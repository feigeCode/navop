#include "windows_rdp_probe.h"

#include <windows.h>
#include <winver.h>

#include <atlbase.h>
#include <atlhost.h>

#import "libid:8C11EFA1-92C3-11D1-BC1E-00C04FA31489" \
    raw_interfaces_only, named_guids, no_namespace, exclude("UINT_PTR")
#include "mstscax.tlh"

#include <cstdio>
#include <string>
#include <vector>

namespace {

// MsRdpClient12 CLSID: 945EE98E-B376-4EC2-B2E5-64C9410F93B7
constexpr CLSID kMsRdpClient12Clsid = {
    0x945ee98e,
    0xb376,
    0x4ec2,
    {0xb2, 0xe5, 0x64, 0xc9, 0x41, 0x0f, 0x93, 0xb7},
};

// IMsRdpClientNonScriptable8 IID: B2B3FA47-3F11-4148-AD24-DFF8684A16D0
constexpr IID kMsRdpClientNonScriptable8Iid = {
    0xb2b3fa47,
    0x3f11,
    0x4148,
    {0xad, 0x24, 0xdf, 0xf8, 0x68, 0x4a, 0x16, 0xd0},
};

struct DllVersion {
    WORD major = 0;
    WORD minor = 0;
    WORD build = 0;
    WORD revision = 0;
};

struct ProbeResources {
    bool ole_initialized = false;
    bool atl_initialized = false;
    HWND parent = nullptr;
    HWND host = nullptr;
    CComPtr<IUnknown> container;
    CComPtr<IUnknown> control;

    ~ProbeResources() {
        control.Release();
        container.Release();
        if (host != nullptr) {
            DestroyWindow(host);
        }
        if (parent != nullptr) {
            DestroyWindow(parent);
        }
        if (atl_initialized) {
            AtlAxWinTerm();
        }
        if (ole_initialized) {
            OleUninitialize();
        }
    }
};

void log_hresult(const char* stage, const char* status, HRESULT result) {
    std::printf(
        "windows-rdp-probe stage=%s status=%s hresult=0x%08lx\n",
        stage,
        status,
        static_cast<unsigned long>(result));
}

void log_win32_error(const char* stage) {
    std::printf(
        "windows-rdp-probe stage=%s status=error win32_error=%lu\n",
        stage,
        static_cast<unsigned long>(GetLastError()));
}

std::string utf8_from_wide(const wchar_t* value) {
    if (value == nullptr || value[0] == L'\0') {
        return {};
    }

    const int source_length = lstrlenW(value);
    const int output_length = WideCharToMultiByte(
        CP_UTF8, 0, value, source_length, nullptr, 0, nullptr, nullptr);
    if (output_length <= 0) {
        return {};
    }

    std::string output(static_cast<size_t>(output_length), '\0');
    WideCharToMultiByte(
        CP_UTF8,
        0,
        value,
        source_length,
        output.data(),
        output_length,
        nullptr,
        nullptr);
    return output;
}

HWND create_parent_window() {
    return CreateWindowExW(
        WS_EX_TOOLWINDOW,
        L"STATIC",
        L"navop-windows-rdp-probe",
        WS_POPUP,
        0,
        0,
        1,
        1,
        nullptr,
        nullptr,
        GetModuleHandleW(nullptr),
        nullptr);
}

HWND create_host_window(HWND parent) {
    return CreateWindowExW(
        0,
        ATLAXWIN_CLASSW,
        L"",
        WS_CHILD,
        0,
        0,
        1,
        1,
        parent,
        nullptr,
        GetModuleHandleW(nullptr),
        nullptr);
}

HRESULT create_rdp_control(ProbeResources& resources) {
    LPOLESTR class_id = nullptr;
    HRESULT result = StringFromCLSID(kMsRdpClient12Clsid, &class_id);
    if (FAILED(result)) {
        return result;
    }

    std::wstring control_name = L"CLSID:";
    control_name += class_id;
    CoTaskMemFree(class_id);

    return AtlAxCreateControlEx(
        control_name.c_str(),
        resources.host,
        nullptr,
        &resources.container,
        &resources.control,
        IID_NULL,
        nullptr);
}

bool read_loaded_dll_version(
    DllVersion& version,
    const char*& unavailable_reason) {
    unavailable_reason = "unknown";

    HMODULE module = GetModuleHandleW(L"mstscax.dll");
    if (module == nullptr) {
        unavailable_reason = "module-not-loaded";
        return false;
    }

    wchar_t path[MAX_PATH] = {};
    DWORD path_length = GetModuleFileNameW(module, path, ARRAYSIZE(path));
    if (path_length == 0 || path_length >= ARRAYSIZE(path)) {
        unavailable_reason = "module-path-unavailable";
        return false;
    }

    DWORD ignored = 0;
    DWORD info_size = GetFileVersionInfoSizeW(path, &ignored);
    if (info_size == 0) {
        unavailable_reason = "version-info-size-unavailable";
        return false;
    }

    std::vector<BYTE> info(info_size);
    if (!GetFileVersionInfoW(path, 0, info_size, info.data())) {
        unavailable_reason = "version-info-read-failed";
        return false;
    }

    void* root = nullptr;
    UINT root_size = 0;
    if (!VerQueryValueW(info.data(), L"\\", &root, &root_size) ||
        root_size < sizeof(VS_FIXEDFILEINFO)) {
        unavailable_reason = "fixed-info-unavailable";
        return false;
    }

    const auto* fixed = static_cast<const VS_FIXEDFILEINFO*>(root);
    if (fixed->dwSignature != VS_FFI_SIGNATURE) {
        unavailable_reason = "invalid-fixed-info-signature";
        return false;
    }

    version.major = HIWORD(fixed->dwFileVersionMS);
    version.minor = LOWORD(fixed->dwFileVersionMS);
    version.build = HIWORD(fixed->dwFileVersionLS);
    version.revision = LOWORD(fixed->dwFileVersionLS);
    unavailable_reason = nullptr;
    return true;
}

int inspect_control(IUnknown* control) {
    CComPtr<IMsRdpClient10> client;
    HRESULT result = control->QueryInterface(IID_PPV_ARGS(&client));
    if (FAILED(result)) {
        log_hresult("query-imsrdpclient10", "error", result);
        return 7;
    }

    CComPtr<IUnknown> non_scriptable;
    void* non_scriptable_raw = nullptr;
    result = control->QueryInterface(
        kMsRdpClientNonScriptable8Iid, &non_scriptable_raw);
    const bool has_non_scriptable8 = SUCCEEDED(result);
    if (has_non_scriptable8) {
        non_scriptable.Attach(static_cast<IUnknown*>(non_scriptable_raw));
    } else {
        log_hresult("query-nonscriptable8", "unavailable", result);
    }

    BSTR version = nullptr;
    result = client->get_Version(&version);
    if (FAILED(result)) {
        log_hresult("get-version", "error", result);
        return 8;
    }

    const std::string control_version = utf8_from_wide(version);
    SysFreeString(version);

    DllVersion dll_version;
    const char* dll_version_reason = nullptr;
    const bool has_dll_version =
        read_loaded_dll_version(dll_version, dll_version_reason);
    std::printf(
        "windows-rdp-probe stage=inspect status=ok "
        "control_version=%s nonscriptable8=%s ",
        control_version.empty() ? "unknown" : control_version.c_str(),
        has_non_scriptable8 ? "available" : "unavailable");
    if (has_dll_version) {
        std::printf(
            "dll_version=%u.%u.%u.%u\n",
            static_cast<unsigned int>(dll_version.major),
            static_cast<unsigned int>(dll_version.minor),
            static_cast<unsigned int>(dll_version.build),
            static_cast<unsigned int>(dll_version.revision));
    } else {
        std::printf(
            "dll_version=unavailable dll_version_reason=%s\n",
            dll_version_reason == nullptr ? "unknown" : dll_version_reason);
    }
    return 0;
}

int run_probe(ProbeResources& resources) {
    HRESULT result = OleInitialize(nullptr);
    if (FAILED(result)) {
        log_hresult("ole-initialize", "error", result);
        return 1;
    }
    resources.ole_initialized = true;

    if (!AtlAxWinInit()) {
        log_win32_error("atlaxwin-init");
        return 2;
    }
    resources.atl_initialized = true;

    resources.parent = create_parent_window();
    if (resources.parent == nullptr) {
        log_win32_error("create-parent");
        return 3;
    }

    resources.host = create_host_window(resources.parent);
    if (resources.host == nullptr) {
        log_win32_error("create-host");
        return 4;
    }

    result = create_rdp_control(resources);
    if (FAILED(result)) {
        log_hresult("create-msrdpclient12", "unavailable", result);
        return 6;
    }

    return inspect_control(resources.control);
}

}  // namespace

extern "C" int32_t windows_rdp_probe_run(void) {
    ProbeResources resources;
    const int result = run_probe(resources);
    if (result == 0) {
        std::printf("windows-rdp-probe stage=complete status=ok\n");
    }
    return result;
}
