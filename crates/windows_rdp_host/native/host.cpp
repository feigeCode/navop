#include "host_internal.h"

#include <windows.h>

#include <new>

namespace {

NavopRdpResult validate_struct_size(
    uint32_t struct_size,
    uint32_t required_size) noexcept {
    if (struct_size < required_size) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult validate_abi_version(uint32_t abi_version) noexcept {
    if (abi_version != NAVOP_RDP_ABI_VERSION) {
        return NAVOP_RDP_RESULT_ABI_MISMATCH;
    }
    return NAVOP_RDP_RESULT_OK;
}

uint64_t join_generation(uint32_t low, uint32_t high) noexcept {
    return static_cast<uint64_t>(low) |
        (static_cast<uint64_t>(high) << 32U);
}

NavopRdpResult validate_last_error_output(
    NavopRdpLastError* out_error) noexcept {
    if (out_error == nullptr) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    NavopRdpResult result = validate_struct_size(
        out_error->struct_size,
        NAVOP_RDP_LAST_ERROR_LEGACY_SIZE);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    return validate_abi_version(out_error->abi_version);
}

void write_last_error(
    NavopRdpLastError* out_error,
    NavopRdpResult result,
    int32_t hresult,
    uint32_t has_hresult,
    uint32_t stage = NAVOP_RDP_CREATE_STAGE_NONE,
    uint32_t win32_code = UINT32_C(0),
    uint32_t has_win32_code = UINT32_C(0)) noexcept {
    const uint32_t caller_size = out_error->struct_size;
    out_error->struct_size = caller_size;
    out_error->abi_version = NAVOP_RDP_ABI_VERSION;
    out_error->result = result;
    out_error->hresult = hresult;
    out_error->has_hresult = has_hresult;
    out_error->reserved = UINT32_C(0);
    if (caller_size >=
        offsetof(NavopRdpLastError, stage) + sizeof(out_error->stage)) {
        out_error->stage = stage;
    }
    if (caller_size >=
        offsetof(NavopRdpLastError, win32_code) +
            sizeof(out_error->win32_code)) {
        out_error->win32_code = win32_code;
    }
    if (caller_size >=
        offsetof(NavopRdpLastError, has_win32_code) +
            sizeof(out_error->has_win32_code)) {
        out_error->has_win32_code = has_win32_code;
    }
}

}  // namespace

NativeRdpHost::~NativeRdpHost() noexcept {
    destroy_active_x_resources(active_x_resources);
    active_x_resources = nullptr;
}

void clear_last_error(NativeRdpHost* host) noexcept {
    if (host == nullptr) {
        return;
    }
    host->last_result = NAVOP_RDP_RESULT_OK;
    host->last_hresult = 0;
    host->has_last_hresult = UINT32_C(0);
    host->last_stage = NAVOP_RDP_CREATE_STAGE_NONE;
    host->last_win32_code = UINT32_C(0);
    host->has_last_win32_code = UINT32_C(0);
}

NavopRdpResult record_last_diagnostic(
    NativeRdpHost* host,
    NavopRdpResult result,
    uint32_t stage,
    int32_t hresult,
    uint32_t has_hresult,
    uint32_t win32_code,
    uint32_t has_win32_code) noexcept {
    if (host != nullptr) {
        host->last_result = result;
        host->last_hresult = hresult;
        host->has_last_hresult = has_hresult;
        host->last_stage = stage;
        host->last_win32_code = win32_code;
        host->has_last_win32_code = has_win32_code;
    }
    return result;
}

NavopRdpResult record_last_error(
    NativeRdpHost* host,
    NavopRdpResult result) noexcept {
    return record_last_diagnostic(
        host,
        result,
        NAVOP_RDP_CREATE_STAGE_NONE,
        0,
        UINT32_C(0),
        UINT32_C(0),
        UINT32_C(0));
}

NavopRdpResult record_last_hresult(
    NativeRdpHost* host,
    NavopRdpResult result,
    int32_t hresult) noexcept {
    return record_last_diagnostic(
        host,
        result,
        NAVOP_RDP_CREATE_STAGE_NONE,
        hresult,
        UINT32_C(1),
        UINT32_C(0),
        UINT32_C(0));
}

NavopRdpResult record_last_stage_error(
    NativeRdpHost* host,
    NavopRdpResult result,
    uint32_t stage) noexcept {
    return record_last_diagnostic(
        host,
        result,
        stage,
        0,
        UINT32_C(0),
        UINT32_C(0),
        UINT32_C(0));
}

NavopRdpResult record_last_stage_hresult(
    NativeRdpHost* host,
    NavopRdpResult result,
    uint32_t stage,
    int32_t hresult) noexcept {
    return record_last_diagnostic(
        host,
        result,
        stage,
        hresult,
        UINT32_C(1),
        UINT32_C(0),
        UINT32_C(0));
}

NavopRdpResult record_last_stage_win32(
    NativeRdpHost* host,
    NavopRdpResult result,
    uint32_t stage,
    uint32_t win32_code) noexcept {
    return record_last_diagnostic(
        host,
        result,
        stage,
        0,
        UINT32_C(0),
        win32_code,
        win32_code == ERROR_SUCCESS ? UINT32_C(0) : UINT32_C(1));
}

extern "C" NavopRdpResult navop_rdp_probe(
    const NavopRdpProbeOptions* options,
    NavopRdpProbeResult* out_result) noexcept {
    try {
        if (options == nullptr || out_result == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        const uint32_t caller_result_size = out_result->struct_size;

        NavopRdpResult result = validate_struct_size(
            options->struct_size,
            static_cast<uint32_t>(sizeof(NavopRdpProbeOptions)));
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        result = validate_abi_version(options->abi_version);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        result = validate_struct_size(
            caller_result_size,
            static_cast<uint32_t>(sizeof(NavopRdpProbeResult)));
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        result = validate_abi_version(out_result->abi_version);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        out_result->struct_size = caller_result_size;
        out_result->abi_version = NAVOP_RDP_ABI_VERSION;
        out_result->available = UINT32_C(1);
        out_result->reserved = UINT32_C(0);
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}

extern "C" NavopRdpResult navop_rdp_create(
    const NavopRdpCreateOptions* options,
    NativeRdpHost** out_host) noexcept {
    try {
        if (out_host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        *out_host = nullptr;

        if (options == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        NavopRdpResult result = validate_struct_size(
            options->struct_size,
            static_cast<uint32_t>(sizeof(NavopRdpCreateOptions)));
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        result = validate_abi_version(options->abi_version);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        const uint64_t generation =
            join_generation(options->generation_low, options->generation_high);
        NativeRdpHost* host = new (std::nothrow) NativeRdpHost{
            generation,
            static_cast<uint32_t>(GetCurrentThreadId()),
            UINT32_C(0),
            nullptr,
            nullptr,
            CallbackState::Open,
            nullptr,
            NAVOP_RDP_RESULT_OK,
            0,
            UINT32_C(0),
            NAVOP_RDP_CREATE_STAGE_NONE,
            UINT32_C(0),
            UINT32_C(0)};
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_ALLOCATION_FAILED;
        }

        *out_host = host;
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}

extern "C" NavopRdpResult navop_rdp_create_with_parent(
    const NavopRdpCreateWithParentOptions* options,
    NativeRdpHost** out_host) noexcept {
    NavopRdpLastError ignored_error{
        static_cast<uint32_t>(sizeof(NavopRdpLastError)),
        NAVOP_RDP_ABI_VERSION,
        NAVOP_RDP_RESULT_OK,
        0,
        UINT32_C(0),
        UINT32_C(0),
        NAVOP_RDP_CREATE_STAGE_NONE,
        UINT32_C(0),
        UINT32_C(0)};
    return navop_rdp_create_with_parent_v2(
        options,
        out_host,
        &ignored_error);
}

extern "C" NavopRdpResult navop_rdp_create_with_parent_v2(
    const NavopRdpCreateWithParentOptions* options,
    NativeRdpHost** out_host,
    NavopRdpLastError* out_error) noexcept {
    try {
        NavopRdpResult result = validate_last_error_output(out_error);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
        write_last_error(
            out_error,
            NAVOP_RDP_RESULT_OK,
            0,
            UINT32_C(0));

        if (out_host == nullptr) {
            write_last_error(
                out_error,
                NAVOP_RDP_RESULT_INVALID_ARGUMENT,
                0,
                UINT32_C(0));
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        *out_host = nullptr;

        if (options == nullptr) {
            write_last_error(
                out_error,
                NAVOP_RDP_RESULT_INVALID_ARGUMENT,
                0,
                UINT32_C(0));
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        result = validate_struct_size(
            options->struct_size,
            static_cast<uint32_t>(sizeof(NavopRdpCreateWithParentOptions)));
        if (result != NAVOP_RDP_RESULT_OK) {
            write_last_error(out_error, result, 0, UINT32_C(0));
            return result;
        }

        if (options->abi_version != NAVOP_RDP_CREATE_WITH_PARENT_ABI_VERSION) {
            write_last_error(
                out_error,
                NAVOP_RDP_RESULT_ABI_MISMATCH,
                0,
                UINT32_C(0));
            return NAVOP_RDP_RESULT_ABI_MISMATCH;
        }

        if (options->parent_hwnd == 0) {
            write_last_error(
                out_error,
                NAVOP_RDP_RESULT_INVALID_ARGUMENT,
                0,
                UINT32_C(0));
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        const HWND parent = reinterpret_cast<HWND>(options->parent_hwnd);
        if (!IsWindow(parent)) {
            write_last_error(
                out_error,
                NAVOP_RDP_RESULT_INVALID_ARGUMENT,
                0,
                UINT32_C(0));
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        const DWORD parent_thread = GetWindowThreadProcessId(parent, nullptr);
        if (parent_thread == 0 ||
            parent_thread != static_cast<DWORD>(GetCurrentThreadId())) {
            write_last_error(
                out_error,
                NAVOP_RDP_RESULT_WRONG_THREAD,
                0,
                UINT32_C(0));
            return NAVOP_RDP_RESULT_WRONG_THREAD;
        }

        const uint64_t generation =
            join_generation(options->generation_low, options->generation_high);
        NativeRdpHost* host = new (std::nothrow) NativeRdpHost{
            generation,
            static_cast<uint32_t>(GetCurrentThreadId()),
            UINT32_C(0),
            nullptr,
            nullptr,
            CallbackState::Open,
            nullptr,
            NAVOP_RDP_RESULT_OK,
            0,
            UINT32_C(0),
            NAVOP_RDP_CREATE_STAGE_NONE,
            UINT32_C(0),
            UINT32_C(0)};
        if (host == nullptr) {
            write_last_error(
                out_error,
                NAVOP_RDP_RESULT_ALLOCATION_FAILED,
                0,
                UINT32_C(0));
            return NAVOP_RDP_RESULT_ALLOCATION_FAILED;
        }

        result = create_active_x_resources(
            host,
            options->parent_hwnd,
            &host->active_x_resources);
        if (result != NAVOP_RDP_RESULT_OK) {
            write_last_error(
                out_error,
                host->last_result,
                host->last_hresult,
                host->has_last_hresult,
                host->last_stage,
                host->last_win32_code,
                host->has_last_win32_code);
            delete host;
            return result;
        }

        *out_host = host;
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        if (out_error != nullptr &&
            out_error->struct_size >= NAVOP_RDP_LAST_ERROR_LEGACY_SIZE &&
            out_error->abi_version == NAVOP_RDP_ABI_VERSION) {
            write_last_error(
                out_error,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                0,
                UINT32_C(0),
                NAVOP_RDP_CREATE_STAGE_EXCEPTION);
        }
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}

extern "C" NavopRdpResult navop_rdp_get_last_error(
    NativeRdpHost* host,
    NavopRdpLastError* out_error) noexcept {
    try {
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        const NavopRdpResult owner_result = ensure_owner_thread(host);
        if (owner_result != NAVOP_RDP_RESULT_OK) {
            return owner_result;
        }
        const NavopRdpResult output_result =
            validate_last_error_output(out_error);
        if (output_result != NAVOP_RDP_RESULT_OK) {
            return output_result;
        }
        write_last_error(
            out_error,
            host->last_result,
            host->last_hresult,
            host->has_last_hresult,
            host->last_stage,
            host->last_win32_code,
            host->has_last_win32_code);
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}

extern "C" NavopRdpResult navop_rdp_set_bounds(
    NativeRdpHost* host,
    const NavopRdpBounds* bounds) noexcept {
    try {
        // Validation is split after owner-thread admission so a valid host
        // replaces stale diagnostics. Legacy shape:
        // host == nullptr || bounds == nullptr ||
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        const NavopRdpResult owner_result = ensure_owner_thread(host);
        if (owner_result != NAVOP_RDP_RESULT_OK) {
            return owner_result;
        }
        clear_last_error(host);
        if (bounds == nullptr || bounds->width < 0 || bounds->height < 0) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        if (host->callback_state != CallbackState::Open) {
            return record_last_error(
                host,
                NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        return record_last_error(
            host,
            set_active_x_bounds(host->active_x_resources, *bounds));
    } catch (...) {
        return record_last_error(host, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
}

extern "C" NavopRdpResult navop_rdp_update_session_display_settings(
    NativeRdpHost* host,
    const NavopRdpSessionDisplaySettings* settings) noexcept {
    try {
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        const NavopRdpResult owner_result = ensure_owner_thread(host);
        if (owner_result != NAVOP_RDP_RESULT_OK) {
            return owner_result;
        }
        clear_last_error(host);
        if (settings == nullptr) {
            return record_last_error(
                host,
                NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        NavopRdpResult result = validate_struct_size(
            settings->struct_size,
            static_cast<uint32_t>(
                sizeof(NavopRdpSessionDisplaySettings)));
        if (result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(host, result);
        }
        if (settings->abi_version !=
            NAVOP_RDP_SESSION_DISPLAY_SETTINGS_ABI_VERSION) {
            return record_last_error(
                host,
                NAVOP_RDP_RESULT_ABI_MISMATCH);
        }
        if (settings->desktop_width == UINT32_C(0) ||
            settings->desktop_height == UINT32_C(0) ||
            settings->physical_width == UINT32_C(0) ||
            settings->physical_height == UINT32_C(0) ||
            settings->desktop_scale_factor == UINT32_C(0) ||
            settings->device_scale_factor == UINT32_C(0)) {
            return record_last_error(
                host,
                NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        if (host->callback_state != CallbackState::Open) {
            return record_last_error(
                host,
                NAVOP_RDP_RESULT_INVALID_STATE);
        }
        return update_active_x_session_display_settings(
            host,
            host->active_x_resources,
            *settings);
    } catch (...) {
        return record_last_error(host, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
}

extern "C" NavopRdpResult navop_rdp_set_visible(
    NativeRdpHost* host,
    uint32_t visible) noexcept {
    try {
        // Legacy validation shape: host == nullptr || visible > UINT32_C(1)
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        const NavopRdpResult owner_result = ensure_owner_thread(host);
        if (owner_result != NAVOP_RDP_RESULT_OK) {
            return owner_result;
        }
        clear_last_error(host);
        if (visible > UINT32_C(1)) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        if (host->callback_state != CallbackState::Open) {
            return record_last_error(
                host,
                NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        return record_last_error(
            host,
            set_active_x_visible(
                host->active_x_resources,
                visible == UINT32_C(1)));
    } catch (...) {
        return record_last_error(host, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
}

extern "C" NavopRdpResult navop_rdp_focus(
    NativeRdpHost* host) noexcept {
    try {
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        const NavopRdpResult owner_result = ensure_owner_thread(host);
        if (owner_result != NAVOP_RDP_RESULT_OK) {
            return owner_result;
        }
        clear_last_error(host);
        if (host->callback_state != CallbackState::Open) {
            return record_last_error(
                host,
                NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        return record_last_error(
            host,
            focus_active_x(host->active_x_resources));
    } catch (...) {
        return record_last_error(host, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
}

extern "C" NavopRdpResult navop_rdp_register_event_callback(
    NativeRdpHost* host,
    const NavopRdpEventCallbackOptions* options,
    NavopRdpEventCallback callback,
    void* callback_context) noexcept {
    try {
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        NavopRdpResult result = ensure_owner_thread(host);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
        clear_last_error(host);
        if (options == nullptr || callback == nullptr) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }

        result = validate_struct_size(
            options->struct_size,
            static_cast<uint32_t>(sizeof(NavopRdpEventCallbackOptions)));
        if (result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(host, result);
        }

        result = validate_abi_version(options->abi_version);
        if (result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(host, result);
        }

        const uint64_t generation =
            join_generation(options->generation_low, options->generation_high);
        if (generation != host->generation ||
            host->callback_state != CallbackState::Open ||
            host->callback != nullptr) {
            return record_last_error(
                host,
                NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }

        host->callback = callback;
        host->callback_context = callback_context;
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return record_last_error(host, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
}

extern "C" NavopRdpResult navop_rdp_unregister_event_callback(
    NativeRdpHost* host) noexcept {
    try {
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        const NavopRdpResult owner_result = ensure_owner_thread(host);
        if (owner_result != NAVOP_RDP_RESULT_OK) {
            return owner_result;
        }
        clear_last_error(host);

        const NavopRdpResult close_result = close_callback_gate(host);
        if (close_result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(host, close_result);
        }
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return record_last_error(host, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
}

extern "C" NavopRdpResult navop_rdp_destroy(NativeRdpHost** host) noexcept {
    try {
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        if (*host == nullptr) {
            return NAVOP_RDP_RESULT_OK;
        }

        NativeRdpHost* owned = *host;
        const NavopRdpResult owner_result = ensure_owner_thread(owned);
        if (owner_result != NAVOP_RDP_RESULT_OK) {
            return owner_result;
        }
        clear_last_error(owned);

        NavopRdpResult close_result = close_callback_gate(owned);
        if (close_result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(owned, close_result);
        }
        // Clearing the caller handle transfers ownership to this success path.
        // All error paths above preserve the handle and native allocation so
        // the caller can safely retry.
        *host = nullptr;
        delete owned;
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}

extern "C" NavopRdpResult navop_rdp_test_set_last_error(
    NativeRdpHost* host,
    NavopRdpResult result,
    uint32_t stage,
    uint32_t has_hresult,
    int32_t hresult,
    uint32_t has_win32_code,
    uint32_t win32_code) noexcept {
    if (host == nullptr ||
        has_hresult > UINT32_C(1) ||
        has_win32_code > UINT32_C(1)) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    const NavopRdpResult owner_result = ensure_owner_thread(host);
    if (owner_result != NAVOP_RDP_RESULT_OK) {
        return owner_result;
    }
    return record_last_diagnostic(
        host,
        result,
        stage,
        hresult,
        has_hresult,
        win32_code,
        has_win32_code);
}
