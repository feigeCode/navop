#include "connection_policy_internal.h"

#include <atlbase.h>

namespace {

void trace_normalized_policy(
    const NavopRdpConnectionOptions& options) noexcept {
    trace_native_win32("connect.policy.display_mode", options.display_mode);
    trace_native_win32("connect.policy.display_flags", options.display_flags);
    trace_native_win32("connect.policy.desktop_scale", options.desktop_scale_factor);
    trace_native_win32("connect.policy.device_scale", options.device_scale_factor);
    trace_native_win32("connect.policy.resource_flags", options.resource_flags);
    trace_native_win32("connect.policy.audio_mode", options.audio_mode);
    trace_native_win32("connect.policy.audio_quality", options.audio_quality);
    trace_native_win32("connect.policy.audio_flags", options.audio_flags);
    trace_native_win32("connect.policy.keyboard_hook", options.keyboard_hook_mode);
    trace_native_win32("connect.policy.input_flags", options.input_flags);
    trace_native_win32("connect.policy.performance_preset", options.performance_preset);
    trace_native_win32("connect.policy.performance_flags", options.performance_flags);
    trace_native_win32("connect.policy.network_type", options.network_connection_type);
    trace_native_win32("connect.policy.security_flags", options.security_flags);
    trace_native_win32("connect.policy.authentication", options.authentication_level);
    trace_native_win32("connect.policy.gateway_mode", options.gateway_mode);
    trace_native_win32("connect.policy.gateway_flags", options.gateway_flags);
    trace_native_win32("connect.policy.gateway_credentials", options.gateway_credential_source);
    trace_native_win32("connect.policy.gateway_hostname_len", options.gateway_hostname.len);
    trace_native_win32("connect.policy.keep_alive_seconds", options.keep_alive_seconds);
    trace_native_win32("connect.policy.timeout_seconds", options.timeout_seconds);
    trace_native_win32("connect.policy.connection_flags", options.connection_flags);
    trace_native_win32("connect.policy.max_reconnect", options.max_reconnect_attempts);
}

NavopRdpResult required_dispatch_result(
    NativeRdpHost* owner,
    const NativeRdpDispatchTarget& target,
    HRESULT result) noexcept {
    trace_native_hresult(target.trace_stage, static_cast<int32_t>(result));
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    return NAVOP_RDP_RESULT_OK;
}

}  // namespace

NavopRdpResult get_required_dispatch_object(
    NativeRdpHost* owner,
    const NativeRdpDispatchTarget& target,
    IUnknown** out_object) noexcept {
    trace_native_stage(target.trace_stage);
    const HRESULT result = get_dispatch_object(
        target.object,
        target.property_name,
        out_object);
    return required_dispatch_result(owner, target, result);
}

NavopRdpResult set_required_dispatch_bool(
    NativeRdpHost* owner,
    const NativeRdpDispatchTarget& target,
    bool value) noexcept {
    trace_native_stage(target.trace_stage);
    const HRESULT result = set_dispatch_bool(
        target.object,
        target.property_name,
        value);
    return required_dispatch_result(owner, target, result);
}

NavopRdpResult set_required_dispatch_long(
    NativeRdpHost* owner,
    const NativeRdpDispatchTarget& target,
    LONG value) noexcept {
    trace_native_stage(target.trace_stage);
    const HRESULT result = set_dispatch_long(
        target.object,
        target.property_name,
        value);
    return required_dispatch_result(owner, target, result);
}

NavopRdpResult set_required_dispatch_utf16(
    NativeRdpHost* owner,
    const NativeRdpDispatchTarget& target,
    NavopRdpBorrowedUtf16 value) noexcept {
    trace_native_stage(target.trace_stage);
    const HRESULT result = set_dispatch_utf16(
        target.object,
        target.property_name,
        value);
    return required_dispatch_result(owner, target, result);
}

void set_best_effort_dispatch_bool(
    const NativeRdpDispatchTarget& target,
    bool value) noexcept {
    trace_native_stage(target.trace_stage);
    const HRESULT result = set_dispatch_bool(
        target.object,
        target.property_name,
        value);
    trace_native_hresult(target.trace_stage, static_cast<int32_t>(result));
}

NavopRdpResult configure_active_x_connection_policy(
    const NativeRdpConnectionPolicyContext& context,
    const NavopRdpConnectionOptions& options) noexcept {
    trace_normalized_policy(options);
    using ConfigureSection = NavopRdpResult (*)(
        const NativeRdpConnectionPolicyContext&,
        const NavopRdpConnectionOptions&);
    const ConfigureSection sections[] = {
        configure_security_policy,
        configure_reconnect_policy,
        configure_input_policy,
        configure_resource_policy,
    };
    for (const ConfigureSection section : sections) {
        const NavopRdpResult result = section(context, options);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
    }

    NavopRdpResult result = configure_audio_redirection(
        context.owner,
        context.client,
        options);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    result = configure_display_policy(context, options);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    result = configure_performance_policy(context, options);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    return configure_gateway_policy(context, options);
}
