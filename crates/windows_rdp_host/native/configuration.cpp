#include "host_internal.h"

#include <cstdint>
#include <cstring>
#include <limits>

namespace {

NavopRdpResult validate_abi_version(uint32_t abi_version) noexcept {
    if (abi_version != NAVOP_RDP_ABI_VERSION) {
        return NAVOP_RDP_RESULT_ABI_MISMATCH;
    }
    return NAVOP_RDP_RESULT_OK;
}

bool valid_color_depth(int32_t color_depth) noexcept {
    return color_depth == 8 ||
        color_depth == 15 ||
        color_depth == 16 ||
        color_depth == 24 ||
        color_depth == 32;
}

template <typename Field>
bool connection_field_available(
    uint32_t struct_size,
    size_t field_offset) noexcept {
    return static_cast<size_t>(struct_size) >= field_offset + sizeof(Field);
}

template <typename Field>
Field read_connection_field(
    const NavopRdpConnectionOptions* options,
    size_t field_offset) noexcept {
    Field value{};
    std::memcpy(
        &value,
        reinterpret_cast<const uint8_t*>(options) + field_offset,
        sizeof(value));
    return value;
}

bool valid_borrowed_utf16(NavopRdpBorrowedUtf16 text) noexcept {
    return text.len == UINT32_C(0) || text.data != nullptr;
}

bool contains_embedded_nul(NavopRdpBorrowedUtf16 text) noexcept {
    for (uint32_t index = 0; index < text.len; ++index) {
        if (text.data[index] == 0) {
            return true;
        }
    }
    return false;
}

NavopRdpConnectionOptions current_connection_defaults(
    NavopRdpBorrowedUtf16 host,
    uint32_t port,
    int32_t desktop_width,
    int32_t desktop_height,
    int32_t color_depth,
    uint32_t flags) noexcept {
    NavopRdpConnectionOptions normalized{};
    normalized.struct_size =
        static_cast<uint32_t>(sizeof(NavopRdpConnectionOptions));
    normalized.abi_version = NAVOP_RDP_ABI_VERSION;
    normalized.host = host;
    normalized.port = port;
    normalized.desktop_width = desktop_width;
    normalized.desktop_height = desktop_height;
    normalized.color_depth = color_depth;
    normalized.flags = flags;
    normalized.legacy_reserved = UINT32_C(0);
    normalized.display_mode = NAVOP_RDP_DISPLAY_MODE_DYNAMIC;
    normalized.display_flags = UINT32_C(0);
    normalized.desktop_scale_factor = UINT32_C(100);
    normalized.device_scale_factor = UINT32_C(100);
    normalized.resource_flags = NAVOP_RDP_RESOURCE_FLAG_CLIPBOARD;
    normalized.audio_mode = NAVOP_RDP_AUDIO_MODE_LOCAL;
    normalized.audio_quality = NAVOP_RDP_AUDIO_QUALITY_DYNAMIC;
    normalized.audio_flags = UINT32_C(0);
    normalized.keyboard_hook_mode = NAVOP_RDP_KEYBOARD_HOOK_REMOTE;
    normalized.input_flags = NAVOP_RDP_INPUT_FLAGS_KNOWN;
    normalized.performance_preset = NAVOP_RDP_PERFORMANCE_PRESET_AUTO;
    normalized.performance_flags = NAVOP_RDP_PERFORMANCE_FLAGS_KNOWN;
    normalized.network_connection_type =
        NAVOP_RDP_NETWORK_CONNECTION_AUTODETECT;
    normalized.security_flags =
        NAVOP_RDP_SECURITY_FLAG_ENABLE_CREDSSP |
        NAVOP_RDP_SECURITY_FLAG_ENCRYPTION_ENABLED;
    normalized.authentication_level =
        NAVOP_RDP_AUTHENTICATION_LEVEL_CONNECT;
    normalized.gateway_mode = NAVOP_RDP_GATEWAY_MODE_NONE;
    normalized.gateway_flags = NAVOP_RDP_GATEWAY_FLAG_BYPASS_LOCAL;
    normalized.gateway_credential_source =
        NAVOP_RDP_GATEWAY_CREDENTIAL_PASSWORD;
    normalized.gateway_hostname = NavopRdpBorrowedUtf16 {nullptr, 0};
    normalized.keep_alive_seconds = UINT32_C(60);
    normalized.timeout_seconds = UINT32_C(600);
    normalized.connection_flags =
        NAVOP_RDP_CONNECTION_POLICY_FLAG_AUTO_RECONNECT;
    normalized.max_reconnect_attempts = NAVOP_RDP_MAX_RECONNECT_ATTEMPTS;
    return normalized;
}

NavopRdpResult normalize_connection_options(
    const NavopRdpConnectionOptions* options,
    NavopRdpConnectionOptions* out_options) noexcept {
    if (options == nullptr || out_options == nullptr) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    uint32_t struct_size = UINT32_C(0);
    std::memcpy(&struct_size, options, sizeof(struct_size));
    if (struct_size < NAVOP_RDP_CONNECTION_LEGACY_SIZE) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    const uint32_t abi_version = read_connection_field<uint32_t>(
        options,
        offsetof(NavopRdpConnectionOptions, abi_version));
    NavopRdpResult result = validate_abi_version(abi_version);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    const NavopRdpBorrowedUtf16 host =
        read_connection_field<NavopRdpBorrowedUtf16>(
            options,
            offsetof(NavopRdpConnectionOptions, host));
    const uint32_t port = read_connection_field<uint32_t>(
        options,
        offsetof(NavopRdpConnectionOptions, port));
    const int32_t desktop_width = read_connection_field<int32_t>(
        options,
        offsetof(NavopRdpConnectionOptions, desktop_width));
    const int32_t desktop_height = read_connection_field<int32_t>(
        options,
        offsetof(NavopRdpConnectionOptions, desktop_height));
    const int32_t color_depth = read_connection_field<int32_t>(
        options,
        offsetof(NavopRdpConnectionOptions, color_depth));
    const uint32_t flags = read_connection_field<uint32_t>(
        options,
        offsetof(NavopRdpConnectionOptions, flags));

    NavopRdpConnectionOptions normalized = current_connection_defaults(
        host,
        port,
        desktop_width,
        desktop_height,
        color_depth,
        flags);

#define NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(field) \
    do { \
        if (connection_field_available<decltype(normalized.field)>( \
                struct_size, \
                offsetof(NavopRdpConnectionOptions, field))) { \
            normalized.field = \
                read_connection_field<decltype(normalized.field)>( \
                    options, \
                    offsetof(NavopRdpConnectionOptions, field)); \
        } \
    } while (false)

    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(display_mode);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(display_flags);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(desktop_scale_factor);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(device_scale_factor);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(resource_flags);
    const bool audio_mode_available =
        connection_field_available<uint32_t>(
            struct_size,
            offsetof(NavopRdpConnectionOptions, audio_mode));
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(audio_mode);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(audio_quality);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(audio_flags);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(keyboard_hook_mode);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(input_flags);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(performance_preset);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(performance_flags);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(network_connection_type);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(security_flags);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(authentication_level);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(gateway_mode);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(gateway_flags);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(gateway_credential_source);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(gateway_hostname);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(keep_alive_seconds);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(timeout_seconds);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(connection_flags);
    NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE(max_reconnect_attempts);

#undef NAVOP_COPY_CONNECTION_FIELD_IF_AVAILABLE

    if (!audio_mode_available &&
        (flags & NAVOP_RDP_CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED) != 0) {
        normalized.audio_mode = NAVOP_RDP_AUDIO_MODE_DISABLED;
    }

    *out_options = normalized;
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult validate_connection_options(
    const NavopRdpConnectionOptions& options) noexcept {
    if ((options.flags & ~NAVOP_RDP_CONNECTION_FLAGS_KNOWN) != 0 ||
        (options.display_flags & ~NAVOP_RDP_DISPLAY_FLAGS_KNOWN) != 0 ||
        (options.resource_flags & ~NAVOP_RDP_RESOURCE_FLAGS_KNOWN) != 0 ||
        (options.audio_flags & ~NAVOP_RDP_AUDIO_FLAGS_KNOWN) != 0 ||
        (options.input_flags & ~NAVOP_RDP_INPUT_FLAGS_KNOWN) != 0 ||
        (options.performance_flags & ~NAVOP_RDP_PERFORMANCE_FLAGS_KNOWN) != 0 ||
        (options.security_flags & ~NAVOP_RDP_SECURITY_FLAGS_KNOWN) != 0 ||
        (options.gateway_flags & ~NAVOP_RDP_GATEWAY_FLAGS_KNOWN) != 0 ||
        (options.connection_flags &
         ~NAVOP_RDP_CONNECTION_POLICY_FLAGS_KNOWN) != 0) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    if ((options.display_mode != NAVOP_RDP_DISPLAY_MODE_DYNAMIC &&
         options.display_mode != NAVOP_RDP_DISPLAY_MODE_FIXED) ||
        options.audio_mode > NAVOP_RDP_AUDIO_MODE_DISABLED ||
        options.audio_quality > NAVOP_RDP_AUDIO_QUALITY_HIGH ||
        options.keyboard_hook_mode > NAVOP_RDP_KEYBOARD_HOOK_FULLSCREEN ||
        options.performance_preset > NAVOP_RDP_PERFORMANCE_PRESET_LAN ||
        options.network_connection_type >
            NAVOP_RDP_NETWORK_CONNECTION_AUTODETECT ||
        options.authentication_level > NAVOP_RDP_AUTHENTICATION_LEVEL_REJECT ||
        options.gateway_mode > NAVOP_RDP_GATEWAY_MODE_AUTO_DETECT ||
        (options.gateway_credential_source !=
             NAVOP_RDP_GATEWAY_CREDENTIAL_PASSWORD &&
         options.gateway_credential_source !=
             NAVOP_RDP_GATEWAY_CREDENTIAL_SMART_CARD &&
         options.gateway_credential_source !=
             NAVOP_RDP_GATEWAY_CREDENTIAL_ANY) ||
        options.desktop_scale_factor < UINT32_C(100) ||
        options.desktop_scale_factor > UINT32_C(500) ||
        (options.device_scale_factor != UINT32_C(100) &&
         options.device_scale_factor != UINT32_C(140) &&
         options.device_scale_factor != UINT32_C(180)) ||
        options.keep_alive_seconds >
            static_cast<uint32_t>(
                (std::numeric_limits<int32_t>::max)() / INT32_C(1000)) ||
        options.timeout_seconds >
            static_cast<uint32_t>((std::numeric_limits<int32_t>::max)()) ||
        options.max_reconnect_attempts > NAVOP_RDP_MAX_RECONNECT_ATTEMPTS) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    if (options.host.len == 0 ||
        options.host.len > NAVOP_RDP_MAX_HOST_UTF16_CODE_UNITS ||
        options.host.data == nullptr ||
        contains_embedded_nul(options.host) ||
        options.port == 0 ||
        options.port > UINT32_C(65535) ||
        options.desktop_width <= 0 ||
        options.desktop_height <= 0 ||
        !valid_color_depth(options.color_depth)) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    if (!valid_borrowed_utf16(options.gateway_hostname) ||
        options.gateway_hostname.len >
            NAVOP_RDP_MAX_GATEWAY_HOST_UTF16_CODE_UNITS ||
        (options.gateway_mode == NAVOP_RDP_GATEWAY_MODE_EXPLICIT &&
         options.gateway_hostname.len == UINT32_C(0)) ||
        (options.gateway_hostname.len != UINT32_C(0) &&
         contains_embedded_nul(options.gateway_hostname))) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    return NAVOP_RDP_RESULT_OK;
}

}  // namespace

extern "C" NavopRdpResult navop_rdp_connect(
    NativeRdpHost* host,
    const NavopRdpConnectionOptions* options) noexcept {
    try {
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        NavopRdpResult result = ensure_owner_thread(host);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
        clear_last_error(host);
        if (options == nullptr) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        if (host->callback_state != CallbackState::Open) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }

        NavopRdpConnectionOptions normalized{};
        result = normalize_connection_options(options, &normalized);
        if (result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(host, result);
        }
        result = validate_connection_options(normalized);
        if (result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(host, result);
        }

        return connect_active_x(host, host->active_x_resources, normalized);
    } catch (...) {
        return record_last_error(host, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
}
