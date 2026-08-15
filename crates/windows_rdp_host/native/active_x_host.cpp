#include "host_internal.h"

#include <windows.h>

#include <atlbase.h>
#include <atlhost.h>
#include <ocidl.h>

#pragma warning(push)
#pragma warning(disable : 4471)
#include "mstscax.tlh"
#pragma warning(pop)

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

// Distinguishable failure for "the ActiveX drawing window is not inside the
// host subtree yet". The connection may still complete; callers re-synchronize
// bounds after the next LoginComplete/Reconnected instead of treating this as a
// terminal error. 'NA' is the Navop facility tag.
constexpr HRESULT kPresentationIncompleteHresult =
    MAKE_HRESULT(SEVERITY_ERROR, FACILITY_ITF, 0x4E41);

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
    CComPtr<IMsRdpClientNonScriptable3> non_scriptable3;
    CComPtr<IMsRdpClientNonScriptable5> non_scriptable5;
    NativeRdpEventSubscription* event_subscription = nullptr;

    ~ActiveXCleanup() noexcept {
        destroy_event_subscription(event_subscription);
        event_subscription = nullptr;
        non_scriptable5.Release();
        non_scriptable3.Release();
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

struct PresentationWindowError {
    DWORD code;
    HRESULT result;
};

PresentationWindowError presentation_window_error(DWORD error) noexcept {
    const DWORD code = error == ERROR_SUCCESS
        ? ERROR_INVALID_WINDOW_HANDLE
        : error;
    return PresentationWindowError{
        code,
        code == ERROR_INVALID_WINDOW_HANDLE
            ? kPresentationIncompleteHresult
            : HRESULT_FROM_WIN32(code),
    };
}

HRESULT position_direct_control_window(
    ActiveXCleanup& resources,
    HWND control_window,
    const RECT& client_rect) noexcept {
    const HWND control_parent = GetParent(control_window);
    trace_native_pointer(
        "presentation.control_parent",
        reinterpret_cast<uintptr_t>(control_parent));
    if (control_parent != resources.host_window) {
        // The control window is not a direct child of the native host. The
        // real drawing surface may live deeper in the subtree; never report
        // silent success when it cannot be positioned.
        if (!IsChild(resources.host_window, control_window)) {
            trace_native_stage(
                "presentation.position_control_window.control_not_descendant");
            return kPresentationIncompleteHresult;
        }
        trace_native_stage(
            "presentation.position_control_window.non_direct_descendant");

        RECT mapped_rect = client_rect;
        SetLastError(ERROR_SUCCESS);
        const int mapped_points = MapWindowPoints(
            resources.host_window,
            control_parent,
            reinterpret_cast<POINT*>(&mapped_rect),
            2);
        const DWORD map_error = GetLastError();
        trace_native_win32(
            "presentation.position_control_window.map_points",
            static_cast<uint32_t>(mapped_points));
        if (mapped_points == 0 && map_error != ERROR_SUCCESS) {
            const PresentationWindowError error =
                presentation_window_error(map_error);
            trace_native_win32(
                "presentation.position_control_window.map_failed",
                static_cast<uint32_t>(error.code));
            return error.result;
        }
        UINT descendant_flags = SWP_NOZORDER | SWP_NOACTIVATE;
        if (IsWindowVisible(resources.host_window)) {
            descendant_flags |= SWP_SHOWWINDOW;
        }
        SetLastError(ERROR_SUCCESS);
        if (!SetWindowPos(
                control_window,
                nullptr,
                mapped_rect.left,
                mapped_rect.top,
                mapped_rect.right - mapped_rect.left,
                mapped_rect.bottom - mapped_rect.top,
                descendant_flags)) {
            const PresentationWindowError error =
                presentation_window_error(GetLastError());
            trace_native_win32(
                "presentation.position_control_window.descendant_failed",
                static_cast<uint32_t>(error.code));
            return error.result;
        }
        trace_native_rect(
            "presentation.position_control_window.descendant_mapped",
            static_cast<int32_t>(mapped_rect.left),
            static_cast<int32_t>(mapped_rect.top),
            static_cast<int32_t>(mapped_rect.right),
            static_cast<int32_t>(mapped_rect.bottom));
        return S_OK;
    }

    UINT position_flags = SWP_NOZORDER | SWP_NOACTIVATE;
    if (IsWindowVisible(resources.host_window)) {
        position_flags |= SWP_SHOWWINDOW;
    }
    trace_native_stage("presentation.position_control_window.before");
    SetLastError(ERROR_SUCCESS);
    if (SetWindowPos(
            control_window,
            nullptr,
            0,
            0,
            client_rect.right - client_rect.left,
            client_rect.bottom - client_rect.top,
            position_flags)) {
        trace_native_stage("presentation.position_control_window.after");
        return S_OK;
    }

    const PresentationWindowError error =
        presentation_window_error(GetLastError());
    trace_native_win32(
        "presentation.position_control_window.failed",
        static_cast<uint32_t>(error.code));
    return error.result;
}

struct DescendantTraceContext {
    uint32_t count = 0;
};

BOOL CALLBACK trace_descendant_window(
    HWND window,
    LPARAM context_pointer) noexcept {
    auto* context = reinterpret_cast<DescendantTraceContext*>(
        context_pointer);
    if (context == nullptr) {
        return FALSE;
    }

    wchar_t native_class_name[256]{};
    const int class_name_len = GetClassNameW(
        window,
        native_class_name,
        static_cast<int>(
            sizeof(native_class_name) / sizeof(native_class_name[0])));
    uint16_t class_name[256]{};
    for (int index = 0; index < class_name_len; ++index) {
        class_name[index] = static_cast<uint16_t>(
            native_class_name[index]);
    }
    RECT rect{};
    SetLastError(ERROR_SUCCESS);
    if (!GetWindowRect(window, &rect)) {
        const DWORD last_error = GetLastError();
        trace_native_win32(
            "presentation.control_descendant_rect.failed",
            static_cast<uint32_t>(
                last_error == ERROR_SUCCESS
                    ? ERROR_INVALID_WINDOW_HANDLE
                    : last_error));
    }
    trace_native_window(
        "presentation.control_descendant",
        context->count,
        reinterpret_cast<uintptr_t>(window),
        reinterpret_cast<uintptr_t>(GetParent(window)),
        IsWindowVisible(window) ? 1U : 0U,
        static_cast<uintptr_t>(GetWindowLongPtrW(window, GWL_STYLE)),
        static_cast<uintptr_t>(GetWindowLongPtrW(window, GWL_EXSTYLE)),
        static_cast<int32_t>(rect.left),
        static_cast<int32_t>(rect.top),
        static_cast<int32_t>(rect.right),
        static_cast<int32_t>(rect.bottom),
        class_name,
        class_name_len > 0
            ? static_cast<uint32_t>(class_name_len)
            : UINT32_C(0));
    ++context->count;
    return TRUE;
}

void trace_control_descendants(HWND control_window) noexcept {
    DescendantTraceContext context;
    SetLastError(ERROR_SUCCESS);
    if (!EnumChildWindows(
            control_window,
            trace_descendant_window,
            reinterpret_cast<LPARAM>(&context))) {
        const DWORD error = GetLastError();
        if (error != ERROR_SUCCESS) {
            trace_native_win32(
                "presentation.enumerate_control_descendants.failed",
                static_cast<uint32_t>(error));
        }
    }
    trace_native_win32(
        "presentation.control_descendant_count",
        context.count);
}

void trace_presentation_window_state(
    const ActiveXCleanup& resources,
    HWND control_window) noexcept {
    trace_native_pointer(
        "presentation.host_parent",
        reinterpret_cast<uintptr_t>(GetParent(resources.host_window)));
    trace_native_pointer(
        "presentation.control_root",
        reinterpret_cast<uintptr_t>(
            GetAncestor(control_window, GA_ROOT)));
    trace_native_pointer(
        "presentation.control_root_owner",
        reinterpret_cast<uintptr_t>(
            GetAncestor(control_window, GA_ROOTOWNER)));
    trace_native_win32(
        "presentation.control_is_host_descendant",
        IsChild(resources.host_window, control_window) ? 1U : 0U);
    trace_native_win32(
        "presentation.host_visible",
        IsWindowVisible(resources.host_window) ? 1U : 0U);
    trace_native_win32(
        "presentation.control_visible",
        IsWindowVisible(control_window) ? 1U : 0U);
    const HWND owner = GetParent(resources.host_window);
    if (owner != nullptr) {
        trace_native_win32(
            "presentation.owner_visible",
            IsWindowVisible(owner) ? 1U : 0U);
        trace_native_win32(
            "presentation.owner_iconic",
            IsIconic(owner) ? 1U : 0U);
        trace_native_win32(
            "presentation.owner_dpi",
            static_cast<uint32_t>(GetDpiForWindow(owner)));
    }
    trace_native_win32(
        "presentation.host_dpi",
        static_cast<uint32_t>(GetDpiForWindow(resources.host_window)));
    trace_native_win32(
        "presentation.control_dpi",
        static_cast<uint32_t>(GetDpiForWindow(control_window)));

    RECT host_rect{};
    if (GetWindowRect(resources.host_window, &host_rect)) {
        trace_native_rect(
            "presentation.host_window_rect",
            static_cast<int32_t>(host_rect.left),
            static_cast<int32_t>(host_rect.top),
            static_cast<int32_t>(host_rect.right),
            static_cast<int32_t>(host_rect.bottom));
    }
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
    trace_control_descendants(control_window);
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
        const HRESULT position_result = position_direct_control_window(
            resources,
            control_window,
            client_rect);
        if (FAILED(position_result)) {
            return position_result;
        }
        trace_presentation_window_state(resources, control_window);
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

    trace_native_stage("create.query_non_scriptable3.before");
    const HRESULT non_scriptable3_result =
        resources->state.control->QueryInterface(
            IID_PPV_ARGS(&resources->state.non_scriptable3));
    trace_native_hresult(
        "create.query_non_scriptable3.after",
        static_cast<int32_t>(non_scriptable3_result));
    if (FAILED(non_scriptable3_result) ||
        resources->state.non_scriptable3 == nullptr) {
        if (FAILED(non_scriptable3_result)) {
            return record_last_stage_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                NAVOP_RDP_CREATE_STAGE_QUERY_NON_SCRIPTABLE,
                static_cast<int32_t>(non_scriptable3_result));
        }
        return record_last_stage_error(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_QUERY_NON_SCRIPTABLE);
    }

    trace_native_stage("create.query_non_scriptable5.before");
    const HRESULT non_scriptable5_result =
        resources->state.control->QueryInterface(
            IID_PPV_ARGS(&resources->state.non_scriptable5));
    trace_native_hresult(
        "create.query_non_scriptable5.after",
        static_cast<int32_t>(non_scriptable5_result));
    if (FAILED(non_scriptable5_result) ||
        resources->state.non_scriptable5 == nullptr) {
        if (FAILED(non_scriptable5_result)) {
            return record_last_stage_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                NAVOP_RDP_CREATE_STAGE_QUERY_NON_SCRIPTABLE,
                static_cast<int32_t>(non_scriptable5_result));
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
    if (FAILED(initial_layout_result) &&
        initial_layout_result != kPresentationIncompleteHresult) {
        return record_last_stage_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL,
            static_cast<int32_t>(initial_layout_result));
    }
    if (initial_layout_result == kPresentationIncompleteHresult) {
        // The ActiveX control creates its drawing window during in-place
        // activation, so a 1x1 create-time bounds sync is expected to be
        // incomplete. Never fail the create here: the Rust presentation
        // re-synchronizes bounds after LoginComplete/Reconnected and every
        // set_bounds call re-runs synchronize_control_bounds.
        trace_native_stage("create.synchronize_bounds.deferred");
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
        if (layout_result == kPresentationIncompleteHresult) {
            return NAVOP_RDP_RESULT_PRESENTATION_INCOMPLETE;
        }
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult update_active_x_session_display_settings(
    NativeRdpHost* owner,
    NativeRdpActiveXResources* resources,
    const NavopRdpSessionDisplaySettings& settings) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return record_last_error(owner, resource_result);
    }

    short connected = 0;
    HRESULT result = resources->state.client->get_Connected(&connected);
    trace_native_hresult(
        "display.get_connected.after",
        static_cast<int32_t>(result));
    trace_native_win32(
        "display.connected_state",
        static_cast<uint32_t>(static_cast<uint16_t>(connected)));
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    if (connected != 1) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INVALID_STATE);
    }

    trace_native_win32(
        "display.desktop_width",
        settings.desktop_width);
    trace_native_win32(
        "display.desktop_height",
        settings.desktop_height);
    trace_native_win32(
        "display.physical_width",
        settings.physical_width);
    trace_native_win32(
        "display.physical_height",
        settings.physical_height);
    trace_native_win32(
        "display.orientation",
        settings.orientation);
    trace_native_win32(
        "display.desktop_scale_factor",
        settings.desktop_scale_factor);
    trace_native_win32(
        "display.device_scale_factor",
        settings.device_scale_factor);

    trace_native_stage(
        "display.update_session_display_settings.before");
    result = resources->state.client->UpdateSessionDisplaySettings(
        static_cast<ULONG>(settings.desktop_width),
        static_cast<ULONG>(settings.desktop_height),
        static_cast<ULONG>(settings.physical_width),
        static_cast<ULONG>(settings.physical_height),
        static_cast<ULONG>(settings.orientation),
        static_cast<ULONG>(settings.desktop_scale_factor),
        static_cast<ULONG>(settings.device_scale_factor));
    trace_native_hresult(
        "display.update_session_display_settings.after",
        static_cast<int32_t>(result));

    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
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
            if (layout_result == kPresentationIncompleteHresult) {
                return NAVOP_RDP_RESULT_PRESENTATION_INCOMPLETE;
            }
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

    // All connection policy sections run before Connect with fail-fast
    // semantics: security, reconnect, input, resource, audio, display,
    // performance, gateway (connection_policy.cpp).
    const NativeRdpConnectionPolicyContext policy_context{
        owner,
        resources->state.control,
        resources->state.client,
        resources->state.non_scriptable3,
        resources->state.non_scriptable5,
    };
    const NavopRdpResult policy_result =
        configure_active_x_connection_policy(policy_context, options);
    if (policy_result != NAVOP_RDP_RESULT_OK) {
        return policy_result;
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

NavopRdpResult get_active_x_presentation_state(
    NativeRdpActiveXResources* resources,
    NavopRdpPresentationState* out_state) noexcept {
    if (out_state == nullptr) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    if (out_state->struct_size <
        static_cast<uint32_t>(sizeof(NavopRdpPresentationState))) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    const uint32_t caller_size = out_state->struct_size;
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        // A live host always produces a snapshot: unavailable resources simply
        // report every flag as zero so the caller can treat the presentation
        // as not ready without failing the session.
        *out_state = NavopRdpPresentationState{
            caller_size,
            NAVOP_RDP_PRESENTATION_STATE_ABI_VERSION,
            UINT32_C(0),
            UINT32_C(0),
            UINT32_C(0),
            UINT32_C(0),
            UINT32_C(0),
            UINT32_C(0)};
        return NAVOP_RDP_RESULT_OK;
    }

    HWND control_window = nullptr;
    const HRESULT window_result =
        resources->state.in_place_object->GetWindow(&control_window);
    const bool control_valid =
        SUCCEEDED(window_result) &&
        control_window != nullptr &&
        IsWindow(control_window);

    RECT host_rect{};
    const bool host_rect_nonzero =
        GetWindowRect(resources->state.host_window, &host_rect) &&
        host_rect.right > host_rect.left &&
        host_rect.bottom > host_rect.top;

    RECT control_rect{};
    const bool control_rect_nonzero =
        control_valid &&
        GetWindowRect(control_window, &control_rect) &&
        control_rect.right > control_rect.left &&
        control_rect.bottom > control_rect.top;

    *out_state = NavopRdpPresentationState{
        caller_size,
        NAVOP_RDP_PRESENTATION_STATE_ABI_VERSION,
        control_valid ? UINT32_C(1) : UINT32_C(0),
        host_rect_nonzero ? UINT32_C(1) : UINT32_C(0),
        control_rect_nonzero ? UINT32_C(1) : UINT32_C(0),
        control_valid && IsWindowVisible(control_window) ? UINT32_C(1)
                                                         : UINT32_C(0),
        control_valid &&
                IsChild(resources->state.host_window, control_window)
            ? UINT32_C(1)
            : UINT32_C(0),
        IsWindowVisible(resources->state.host_window) ? UINT32_C(1)
                                                      : UINT32_C(0)};
    trace_native_win32(
        "presentation.state.control_hwnd_valid",
        out_state->control_hwnd_valid);
    trace_native_win32(
        "presentation.state.host_rect_nonzero",
        out_state->host_rect_nonzero);
    trace_native_win32(
        "presentation.state.control_rect_nonzero",
        out_state->control_rect_nonzero);
    trace_native_win32(
        "presentation.state.control_visible",
        out_state->control_visible);
    trace_native_win32(
        "presentation.state.control_is_host_descendant",
        out_state->control_is_host_descendant);
    trace_native_win32(
        "presentation.state.host_visible",
        out_state->host_visible);
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
