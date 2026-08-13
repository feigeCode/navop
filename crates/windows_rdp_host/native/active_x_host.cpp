#include "host_internal.h"

#include <windows.h>

#include <atlbase.h>
#include <atlhost.h>
#include <ocidl.h>

#include "mstscax.tlh"

#include <limits>
#include <memory>
#include <new>

namespace {

constexpr wchar_t kNativeHostWindowClassName[] =
    L"Navop.WindowsRdpHost.Container";

// 1Remote deliberately hosts the broadly deployed
// MsRdpClient9NotSafeForScripting control instead of the newest RDP control.
// Keep the native smoke host on the same conservative control generation
// until the GPUI/ActiveX hosting path is proven on real Windows machines.
constexpr CLSID kMsRdpClient9NotSafeForScriptingClsid = {
    0x8b918b82,
    0x7985,
    0x4c24,
    {0x89, 0xdf, 0xc3, 0x3a, 0xd2, 0xbb, 0xfb, 0xcd},
};

// CAxHostWindow::_CreatorClass constructs a CComObject<CAxHostWindow>.
// CComObject takes an ATL module lock in its constructor, so an executable
// which only links this native code through a Rust static library must still
// provide a live CAtlModule. Without it, ATL dereferences a null _pAtlModule
// inside AtlAxAttachControl before that inline helper can return an HRESULT.
class WindowsRdpAtlModule final :
    public CAtlModuleT<WindowsRdpAtlModule> {};

WindowsRdpAtlModule windows_rdp_atl_module;

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
    CComPtr<IUnknown> container;
    CComPtr<IUnknown> control;
    CComPtr<IOleInPlaceObject> in_place_object;
    CComPtr<IMsRdpClient9> client;
    CComPtr<IMsRdpClientNonScriptable2> non_scriptable;
    NativeRdpEventSubscription* event_subscription = nullptr;

    ~ActiveXCleanup() noexcept {
        destroy_event_subscription(event_subscription);
        event_subscription = nullptr;
        non_scriptable.Release();
        client.Release();
        in_place_object.Release();
        control.Release();
        container.Release();
        if (host_window != nullptr) {
            DestroyWindow(host_window);
        }
        if (atl_initialized) {
            AtlAxWinTerm();
        }
        if (ole_initialized) {
            OleUninitialize();
        }
    }
};

HRESULT attach_control_with_traces(ActiveXCleanup& resources) noexcept {
    // AtlAxAttachControl is an inline wrapper around three distinct ATL
    // operations. Keep them explicit here so a Windows crash identifies
    // whether ATL fails while allocating its host object, exposing
    // IAxWinHostWindow, or activating the RDP control.
    trace_native_pointer(
        "create.atl_module",
        reinterpret_cast<uintptr_t>(ATL::_pAtlModule));
    trace_native_stage("create.atl_container_instance.before");
    HRESULT result = CAxHostWindow::_CreatorClass::CreateInstance(
        nullptr,
        __uuidof(IUnknown),
        reinterpret_cast<void**>(&resources.container));
    trace_native_hresult(
        "create.atl_container_instance.after",
        static_cast<int32_t>(result));
    if (FAILED(result) || resources.container == nullptr) {
        return FAILED(result) ? result : E_UNEXPECTED;
    }

    CComPtr<IAxWinHostWindow> host;
    trace_native_stage("create.query_ax_host.before");
    result = resources.container->QueryInterface(&host);
    trace_native_hresult(
        "create.query_ax_host.after",
        static_cast<int32_t>(result));
    if (FAILED(result) || host == nullptr) {
        return FAILED(result) ? result : E_NOINTERFACE;
    }

    trace_native_stage("create.ax_host_attach.before");
    result = host->AttachControl(
        resources.control,
        resources.host_window);
    trace_native_hresult(
        "create.ax_host_attach.after",
        static_cast<int32_t>(result));
    return result;
}

HRESULT synchronize_control_bounds(ActiveXCleanup& resources) noexcept {
    if (resources.in_place_object == nullptr ||
        resources.host_window == nullptr) {
        return E_UNEXPECTED;
    }

    RECT client_rect{};
    SetLastError(ERROR_SUCCESS);
    trace_native_stage("presentation.get_client_rect.before");
    if (!GetClientRect(resources.host_window, &client_rect)) {
        const DWORD last_error = GetLastError();
        const DWORD win32_code = last_error == ERROR_SUCCESS
            ? ERROR_INVALID_WINDOW_HANDLE
            : last_error;
        trace_native_win32(
            "presentation.get_client_rect.failed",
            static_cast<uint32_t>(win32_code));
        return HRESULT_FROM_WIN32(win32_code);
    }
    trace_native_rect(
        "presentation.host_client_rect",
        static_cast<int32_t>(client_rect.left),
        static_cast<int32_t>(client_rect.top),
        static_cast<int32_t>(client_rect.right),
        static_cast<int32_t>(client_rect.bottom));

    trace_native_stage("presentation.set_object_rects.before");
    const HRESULT layout_result =
        resources.in_place_object->SetObjectRects(
            &client_rect,
            &client_rect);
    trace_native_hresult(
        "presentation.set_object_rects.after",
        static_cast<int32_t>(layout_result));
    if (FAILED(layout_result)) {
        return layout_result;
    }

    HWND control_window = nullptr;
    trace_native_stage("presentation.get_control_window.before");
    const HRESULT window_result =
        resources.in_place_object->GetWindow(&control_window);
    trace_native_hresult(
        "presentation.get_control_window.after",
        static_cast<int32_t>(window_result));
    trace_native_pointer(
        "presentation.control_window",
        reinterpret_cast<uintptr_t>(control_window));
    if (SUCCEEDED(window_result) &&
        control_window != nullptr &&
        IsWindow(control_window)) {
        RECT control_rect{};
        if (GetWindowRect(control_window, &control_rect)) {
            trace_native_rect(
                "presentation.control_window_rect",
                static_cast<int32_t>(control_rect.left),
                static_cast<int32_t>(control_rect.top),
                static_cast<int32_t>(control_rect.right),
                static_cast<int32_t>(control_rect.bottom));
        }
        trace_native_win32(
            "presentation.control_window_style",
            static_cast<uint32_t>(GetWindowLongPtrW(
                control_window,
                GWL_STYLE)));
    }

    trace_native_stage("presentation.redraw.before");
    SetLastError(ERROR_SUCCESS);
    if (!RedrawWindow(
            resources.host_window,
            nullptr,
            nullptr,
            RDW_INVALIDATE | RDW_UPDATENOW | RDW_ALLCHILDREN)) {
        trace_native_win32(
            "presentation.redraw.failed",
            static_cast<uint32_t>(GetLastError()));
    } else {
        trace_native_stage("presentation.redraw.after");
    }
    return S_OK;
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
        resources->state.host_window == nullptr) {
        return NAVOP_RDP_RESULT_UNAVAILABLE;
    }
    if (!IsWindow(resources->state.parent_window) ||
        !IsWindow(resources->state.host_window)) {
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
    // below a dedicated native child so synchronous parent notifications
    // cannot enter GPUI's Rust window procedure directly. AtlAxAttachControl
    // subclasses this window into the ActiveX host after the RDP control has
    // been explicitly constructed and initialized.
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

    trace_native_stage("create.control_instance.before");
    const HRESULT control_result = CoCreateInstance(
        kMsRdpClient9NotSafeForScriptingClsid,
        nullptr,
        CLSCTX_INPROC_SERVER,
        IID_PPV_ARGS(&resources->state.control));
    trace_native_hresult(
        "create.control_instance.after",
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

    trace_native_stage("create.query_non_scriptable.before");
    const HRESULT non_scriptable_result =
        resources->state.control->QueryInterface(
            IID_PPV_ARGS(&resources->state.non_scriptable));
    trace_native_hresult(
        "create.query_non_scriptable.after",
        static_cast<int32_t>(non_scriptable_result));
    if (FAILED(non_scriptable_result) ||
        resources->state.non_scriptable == nullptr) {
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

    // AtlAxAttachControl treats the supplied object as already initialized and
    // deliberately skips IPersistStreamInit::InitNew. WinForms AxHost, which
    // 1Remote uses, initializes a newly created control before it starts the
    // event sinks and activates the control. Reproduce that required step
    // explicitly before handing the object to ATL.
    CComPtr<IPersistStreamInit> persist_stream_init;
    trace_native_stage("create.query_persist_stream_init.before");
    const HRESULT persist_query_result =
        resources->state.control->QueryInterface(
            IID_PPV_ARGS(&persist_stream_init));
    trace_native_hresult(
        "create.query_persist_stream_init.after",
        static_cast<int32_t>(persist_query_result));
    if (FAILED(persist_query_result) || persist_stream_init == nullptr) {
        if (FAILED(persist_query_result)) {
            return record_last_stage_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL,
                static_cast<int32_t>(persist_query_result));
        }
        return record_last_stage_error(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL);
    }

    trace_native_stage("create.initialize_control.before");
    const HRESULT initialize_result = persist_stream_init->InitNew();
    trace_native_hresult(
        "create.initialize_control.after",
        static_cast<int32_t>(initialize_result));
    if (FAILED(initialize_result)) {
        return record_last_stage_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL,
            static_cast<int32_t>(initialize_result));
    }

    // 1Remote registers managed event handlers before EndInit/CreateControl.
    // AxHost turns those handlers into COM event sinks after initialization.
    // Advise after InitNew but before ATL's in-place activation.
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

    const HRESULT attach_result =
        attach_control_with_traces(resources->state);
    if (FAILED(attach_result) || resources->state.container == nullptr) {
        if (FAILED(attach_result)) {
            return record_last_stage_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL,
                static_cast<int32_t>(attach_result));
        }
        return record_last_stage_error(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL);
    }

    trace_native_stage("create.query_in_place_object.before");
    const HRESULT in_place_result =
        resources->state.control->QueryInterface(
            IID_PPV_ARGS(&resources->state.in_place_object));
    trace_native_hresult(
        "create.query_in_place_object.after",
        static_cast<int32_t>(in_place_result));
    if (FAILED(in_place_result) ||
        resources->state.in_place_object == nullptr) {
        if (FAILED(in_place_result)) {
            return record_last_stage_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL,
                static_cast<int32_t>(in_place_result));
        }
        return record_last_stage_error(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL);
    }

    trace_native_stage("create.synchronize_bounds.before");
    const HRESULT initial_layout_result =
        synchronize_control_bounds(resources->state);
    trace_native_hresult(
        "create.synchronize_bounds.after",
        static_cast<int32_t>(initial_layout_result));
    if (FAILED(initial_layout_result)) {
        return record_last_stage_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL,
            static_cast<int32_t>(initial_layout_result));
    }

    trace_native_stage("create.set_ui_parent.before");
    const HRESULT ui_parent_result =
        resources->state.non_scriptable->put_UIParentWindowHandle(
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
    const HRESULT layout_result =
        synchronize_control_bounds(resources->state);
    if (FAILED(layout_result)) {
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
        window_or_descendant_has_focus(resources->state.host_window)) {
        SetFocus(resources->state.parent_window);
    }
    ShowWindow(
        resources->state.host_window,
        visible ? SW_SHOWNA : SW_HIDE);

    if (visible) {
        const HRESULT layout_result =
            synchronize_control_bounds(resources->state);
        if (FAILED(layout_result)) {
            return NAVOP_RDP_RESULT_INTERNAL_ERROR;
        }
    }

    const LONG_PTR host_style = GetWindowLongPtrW(
        resources->state.host_window,
        GWL_STYLE);
    const bool host_is_visible = (host_style & WS_VISIBLE) != 0;
    if (host_is_visible != visible) {
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
         WS_VISIBLE) == 0) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    SetFocus(resources->state.host_window);
    if (!window_or_descendant_has_focus(resources->state.host_window)) {
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
    trace_native_stage("connect.get_advanced_settings.before");
    result = resources->state.client->get_AdvancedSettings2(
        &advanced_settings);
    trace_native_hresult(
        "connect.get_advanced_settings.after",
        static_cast<int32_t>(result));
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

    trace_native_stage("connect.set_encryption.before");
    result = advanced_settings->put_EncryptionEnabled(1);
    trace_native_hresult(
        "connect.set_encryption.after",
        static_cast<int32_t>(result));
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }

    CComPtr<IMsRdpClientAdvancedSettings5> advanced_settings6;
    trace_native_stage("connect.get_advanced_settings6.before");
    result = resources->state.client->get_AdvancedSettings6(
        &advanced_settings6);
    trace_native_hresult(
        "connect.get_advanced_settings6.after",
        static_cast<int32_t>(result));
    if (FAILED(result) || advanced_settings6 == nullptr) {
        if (FAILED(result)) {
            return record_last_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                static_cast<int32_t>(result));
        }
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    trace_native_stage("connect.set_public_mode.before");
    result = advanced_settings6->put_PublicMode(VARIANT_FALSE);
    trace_native_hresult(
        "connect.set_public_mode.after",
        static_cast<int32_t>(result));
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }

    CComPtr<IMsRdpClientAdvancedSettings8> advanced_settings9;
    trace_native_stage("connect.get_advanced_settings9.before");
    result = resources->state.client->get_AdvancedSettings9(
        &advanced_settings9);
    trace_native_hresult(
        "connect.get_advanced_settings9.after",
        static_cast<int32_t>(result));
    if (FAILED(result) || advanced_settings9 == nullptr) {
        if (FAILED(result)) {
            return record_last_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                static_cast<int32_t>(result));
        }
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    trace_native_stage("connect.set_credssp.before");
    result = advanced_settings9->put_EnableCredSspSupport(VARIANT_TRUE);
    trace_native_hresult(
        "connect.set_credssp.after",
        static_cast<int32_t>(result));
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }

    trace_native_stage("connect.set_authentication_level.before");
    result = advanced_settings9->put_AuthenticationLevel(0);
    trace_native_hresult(
        "connect.set_authentication_level.after",
        static_cast<int32_t>(result));
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

        trace_native_stage("credentials.set_password.before");
        result = resources->state.non_scriptable->put_ClearTextPassword(
            password_bstr.get());
        trace_native_hresult(
            "credentials.set_password.after",
            static_cast<int32_t>(result));
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

void trace_active_x_disconnect_description(
    NativeRdpActiveXResources* resources,
    int32_t disconnect_code,
    int32_t extended_code) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        trace_native_result(
            "disconnect.error_description.unavailable",
            resource_result);
        return;
    }

    CComBSTR description;
    const HRESULT result =
        resources->state.client->GetErrorDescription(
            static_cast<UINT>(disconnect_code),
            static_cast<UINT>(extended_code),
            &description);
    trace_native_utf16(
        "disconnect.error_description",
        static_cast<int32_t>(result),
        reinterpret_cast<const uint16_t*>(
            static_cast<BSTR>(description)),
        SUCCEEDED(result) && description.m_str != nullptr
            ? static_cast<uint32_t>(description.Length())
            : UINT32_C(0));
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
