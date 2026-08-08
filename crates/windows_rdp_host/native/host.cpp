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

}  // namespace

NativeRdpHost::~NativeRdpHost() noexcept {
    destroy_active_x_resources(active_x_resources);
    active_x_resources = nullptr;
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
            nullptr};
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
            static_cast<uint32_t>(sizeof(NavopRdpCreateWithParentOptions)));
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        if (options->abi_version != NAVOP_RDP_CREATE_WITH_PARENT_ABI_VERSION) {
            return NAVOP_RDP_RESULT_ABI_MISMATCH;
        }

        if (options->parent_hwnd == 0) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        const HWND parent = reinterpret_cast<HWND>(options->parent_hwnd);
        if (!IsWindow(parent)) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        const DWORD parent_thread = GetWindowThreadProcessId(parent, nullptr);
        if (parent_thread == 0 ||
            parent_thread != static_cast<DWORD>(GetCurrentThreadId())) {
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
            nullptr};
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_ALLOCATION_FAILED;
        }

        result = create_active_x_resources(
            options->parent_hwnd,
            &host->active_x_resources);
        if (result != NAVOP_RDP_RESULT_OK) {
            delete host;
            return result;
        }

        *out_host = host;
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}

extern "C" NavopRdpResult navop_rdp_register_event_callback(
    NativeRdpHost* host,
    const NavopRdpEventCallbackOptions* options,
    NavopRdpEventCallback callback,
    void* callback_context) noexcept {
    try {
        if (host == nullptr || options == nullptr || callback == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        NavopRdpResult result = ensure_owner_thread(host);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        result = validate_struct_size(
            options->struct_size,
            static_cast<uint32_t>(sizeof(NavopRdpEventCallbackOptions)));
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        result = validate_abi_version(options->abi_version);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        const uint64_t generation =
            join_generation(options->generation_low, options->generation_high);
        if (generation != host->generation ||
            host->callback_state != CallbackState::Open ||
            host->callback != nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        host->callback = callback;
        host->callback_context = callback_context;
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
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

        const NavopRdpResult close_result = close_callback_gate(host);
        if (close_result != NAVOP_RDP_RESULT_OK) {
            return close_result;
        }
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
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

        NavopRdpResult close_result = close_callback_gate(owned);
        if (close_result != NAVOP_RDP_RESULT_OK) {
            return close_result;
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
