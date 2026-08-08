#include "host_internal.h"

#include <windows.h>

#include <atlbase.h>
#include <atlhost.h>

#import "libid:8C11EFA1-92C3-11D1-BC1E-00C04FA31489" \
    raw_interfaces_only, named_guids, no_namespace, exclude("UINT_PTR")
#include "mstscax.tlh"

#include <memory>
#include <new>

namespace {

struct ActiveXCleanup {
    bool ole_initialized = false;
    bool atl_initialized = false;
    HWND child_window = nullptr;
    CComPtr<IUnknown> container;
    CComPtr<IUnknown> control;
    CComPtr<IMsRdpClient10> client;

    ~ActiveXCleanup() noexcept {
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

}  // namespace

struct NativeRdpActiveXResources {
    ActiveXCleanup state;
};

NavopRdpResult create_active_x_resources(
    uintptr_t parent_hwnd,
    NativeRdpActiveXResources** out_resources) noexcept {
    if (out_resources == nullptr) {
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

    *out_resources = resources.release();
    return NAVOP_RDP_RESULT_OK;
}

void destroy_active_x_resources(
    NativeRdpActiveXResources* resources) noexcept {
    delete resources;
}
