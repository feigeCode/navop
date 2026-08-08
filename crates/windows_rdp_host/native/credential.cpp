#include "host_internal.h"

#include <windows.h>

#include <cstring>
#include <limits>
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

NavopRdpResult validate_borrowed_secret(
    NavopRdpBorrowedSecret secret) noexcept {
    if (secret.len == UINT32_C(0)) {
        return NAVOP_RDP_RESULT_OK;
    }
    if (secret.data == nullptr) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    const size_t code_unit_count = static_cast<size_t>(secret.len);
    if (code_unit_count >
        (std::numeric_limits<size_t>::max)() / sizeof(uint16_t)) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    return NAVOP_RDP_RESULT_OK;
}

class SensitiveUtf16Buffer {
public:
    SensitiveUtf16Buffer() noexcept
        : data_(nullptr),
          byte_len_(0) {}

    ~SensitiveUtf16Buffer() noexcept {
        reset();
    }

    SensitiveUtf16Buffer(const SensitiveUtf16Buffer&) = delete;
    SensitiveUtf16Buffer& operator=(const SensitiveUtf16Buffer&) = delete;
    SensitiveUtf16Buffer(SensitiveUtf16Buffer&&) = delete;
    SensitiveUtf16Buffer& operator=(SensitiveUtf16Buffer&&) = delete;

    NavopRdpResult copy_from(NavopRdpBorrowedSecret secret) noexcept {
        reset();
        if (secret.len == UINT32_C(0)) {
            return NAVOP_RDP_RESULT_OK;
        }
        if (secret.data == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        const size_t code_unit_count = static_cast<size_t>(secret.len);
        if (code_unit_count >
            (std::numeric_limits<size_t>::max)() / sizeof(uint16_t)) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        const size_t byte_len = code_unit_count * sizeof(uint16_t);
        uint16_t* copied = new (std::nothrow) uint16_t[code_unit_count];
        if (copied == nullptr) {
            return NAVOP_RDP_RESULT_ALLOCATION_FAILED;
        }

        std::memcpy(copied, secret.data, byte_len);
        data_ = copied;
        byte_len_ = byte_len;
        return NAVOP_RDP_RESULT_OK;
    }

private:
    void reset() noexcept {
        if (data_ == nullptr) {
            return;
        }
        SecureZeroMemory(data_, byte_len_);
        delete[] data_;
        data_ = nullptr;
        byte_len_ = 0;
    }

    uint16_t* data_;
    size_t byte_len_;
};

}  // namespace

extern "C" NavopRdpResult navop_rdp_apply_credentials(
    NativeRdpHost* host,
    const NavopRdpCredentialBundle* credentials) noexcept {
    try {
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        NavopRdpResult result = ensure_owner_thread(host);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
        clear_last_error(host);
        if (credentials == nullptr) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }

        result = validate_struct_size(
            credentials->struct_size,
            static_cast<uint32_t>(sizeof(NavopRdpCredentialBundle)));
        if (result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(host, result);
        }

        result = validate_abi_version(credentials->abi_version);
        if (result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(host, result);
        }
        if (credentials->flags != UINT32_C(0)) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        if (host->callback_state != CallbackState::Open) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }

        result = validate_borrowed_secret(credentials->server_password);
        if (result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(host, result);
        }
        result = validate_borrowed_secret(credentials->gateway_password);
        if (result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(host, result);
        }

        SensitiveUtf16Buffer server_password;
        SensitiveUtf16Buffer gateway_password;
        result = server_password.copy_from(credentials->server_password);
        if (result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(host, result);
        }
        result = gateway_password.copy_from(credentials->gateway_password);
        if (result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(host, result);
        }

        // Transport-only slice: no ActiveX property setter is called yet.
        // Both temporary buffers are wiped by RAII on success, failure, or
        // exception before this synchronous ABI call returns.
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return record_last_error(host, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
}
